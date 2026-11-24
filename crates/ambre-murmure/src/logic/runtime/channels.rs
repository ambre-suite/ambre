// ambre-murmure/src/logic/runtime/channels.rs
// ============================================================================
// Murmure channels: route a channel command to its engine, and a remote delta
// to the channel that holds its document. A channel is a set of documents,
// never an entry of `docs` (the Murmure engine spec).
// ============================================================================

use super::*;
use crate::message::{
    ChannelMatches, HeldReply, HeldSend, MessageAt, MurmureCommand, MurmureEvent,
};
use crate::message::{Mention, MentionTarget, Unreadable};
use crate::{
    ChangeTarget, ChannelState, MurmureError, Opening, OpeningChannel, Position, Query, Receipt,
    Sample, Screened, SeekStep, Sent, Signer, ThreadTarget, members_only, screen,
};
use ambre_bus::{Authored, Instant};
use ambre_bus::{ChannelActivity, ChannelDocument, ChannelDocumentKind, DocumentActivity};
use ambre_bus::{MemberInfo, Mentions, OpenFailure, Relay, Seq, SignedDelta, Unread};
use ambre_core::Flag as _;

/// This user's read cursor on one channel document (the Murmure unread spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadCursor {
    /// The relay number of the last update read.
    pub read: Seq<Relay>,
    /// Whether the document holds an update past `read`: the relay's yes or
    /// no, the one thing known of a segment this client does not hold.
    pub active: ambre_bus::Activity,
}

/// What the Logic thread keeps of a channel whose view closed. Its documents
/// are light subscriptions, and their summaries count its new messages.
#[derive(Debug)]
pub(crate) struct ClosedChannel {
    /// The channel's `meta`, whose activity may name a new segment.
    meta: Id<Meta>,
    /// The documents released at close, and the segments watched since,
    /// each a light subscription now.
    documents: Vec<Id<Document>>,
    /// The final segments not held at close, whose cursors a drop forgets.
    finals: Vec<Id<Document>>,
    /// The counts at close, `None` when they were not known then.
    base: Option<Counts>,
    /// The messages and the mentions each document's summaries counted
    /// since the close.
    counted: HashMap<Id<Document>, Counts>,
    /// The horizon of each document's copy in the local store, until a
    /// reopen asks for it.
    copies: HashMap<Id<Document>, Seq<Relay>>,
    /// The copies a reopen is reading back, each with its horizon.
    reading: HashMap<Id<Document>, Seq<Relay>>,
}

/// Every document of an open channel that may carry a read cursor: those it
/// holds, and its final segments.
pub(crate) fn cursored_documents(channel: &ChannelState) -> Vec<Id<Document>> {
    let mut documents = channel.document_ids();
    for segment in channel.final_segments() {
        if !documents.contains(&segment.document_id()) {
            documents.push(segment.document_id());
        }
    }
    documents
}

/// Notes a payload that an opening did not keep. Each caller asks
/// `expects` first, so a refusal means the two answers disagree. A debug
/// trace records it, and nothing else changes.
fn trace_receipt(document_id: Id<Document>, receipt: Receipt) {
    if receipt == Receipt::NotExpected {
        tracing::debug!("Logic: an opening did not expect a payload of {document_id}");
    }
}

/// The open of `document_id`, a document of the channel whose `meta` is
/// `meta`. The caller reads `meta` from the sealed state of the channel, so
/// the log and the links of that `meta` vouch for the key (`SYNC-229`).
pub(crate) fn open_in_channel(meta: Id<Meta>, document_id: Id<Document>) -> SyncCommand {
    SyncCommand::OpenDocument {
        document_id,
        doc_type: DocType::Murmure,
        meta: Some(meta),
    }
}

/// The document a change of a message of `segment` writes. A locked one may
/// first ask the Sync worker to fetch its amendments, or to create them
/// (the Murmure messages spec).
fn change_ask(
    channel: &mut ChannelState,
    channel_id: Id<Channel>,
    segment: Id<Segment>,
    now: Instant<Authored>,
) -> Result<(Id<Document>, Option<SyncCommand>), MurmureError> {
    Ok(match channel.change_target(segment, now)? {
        ChangeTarget::Write(id) => (id, None),
        ChangeTarget::Open(id) => (id, Some(open_in_channel(channel.meta_id(), id))),
        ChangeTarget::Create(id) => (
            id,
            Some(SyncCommand::CreateAmendments {
                channel_id,
                meta: channel.meta_id(),
                segment,
                amendments: id,
            }),
        ),
    })
}

impl ClosedChannel {
    /// A channel this session never opened, counted from its light
    /// subscriptions (the Murmure notifications spec). Nothing is known of
    /// its count, and no copy of it is in the local store.
    pub(crate) fn watched(meta: Id<Meta>, documents: Vec<Id<Document>>) -> Self {
        Self {
            meta,
            documents,
            finals: Vec::new(),
            base: None,
            counted: HashMap::new(),
            copies: HashMap::new(),
            reading: HashMap::new(),
        }
    }

    /// Whether the channel held `document_id` at close.
    pub(crate) fn holds(&self, document_id: Id<Document>) -> bool {
        self.documents.contains(&document_id)
    }

    /// The channel's `meta`.
    pub(crate) fn meta(&self) -> Id<Meta> {
        self.meta
    }

    /// Count a document the channel did not hold at close, a segment
    /// opened since (the Murmure unread spec). Returns whether it is new.
    pub(crate) fn watch(&mut self, document_id: Id<Document>) -> bool {
        if self.holds(document_id) {
            return false;
        }
        self.documents.push(document_id);
        true
    }

    /// Every document whose read cursor the channel may carry.
    pub(crate) fn cursored(&self) -> Vec<Id<Document>> {
        self.documents.iter().chain(&self.finals).copied().collect()
    }

    /// The channel's mark: the count at close plus the messages counted
    /// since, and the mentions of its threads not held. With no count at
    /// close, a counted message or mention marks it active, and none leaves
    /// the mark the view holds: `None`.
    pub(crate) fn mark(
        &self,
        channel_id: Id<Channel>,
        threads: ambre_core::Count<Mentions>,
    ) -> Option<ChannelActivity> {
        let since = self
            .counted
            .values()
            .fold(Counts::default(), Counts::plus)
            .with_mentions(threads);
        match self.base {
            Some(base) => Some(base.plus(&since).activity(channel_id)),
            None => (!since.unread.is_zero()
                || since.mentions != ambre_core::Count::<Mentions>::ZERO)
                .then_some(ChannelActivity {
                    channel_id,
                    active: ambre_bus::Activity::Active,
                    unread: None,
                    mentions: None,
                }),
        }
    }
}

/// The new messages of a channel, and those of them that mention this user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    unread: ambre_core::Count<Unread>,
    mentions: ambre_core::Count<Mentions>,
}

impl Counts {
    fn plus(self, other: &Self) -> Self {
        Self {
            unread: self.unread.plus(other.unread),
            mentions: self.mentions.plus(other.mentions),
        }
    }

    /// The same counts, with the mentions of the threads not held.
    fn with_mentions(self, mentions: ambre_core::Count<Mentions>) -> Self {
        self.plus(&Self {
            unread: ambre_core::Count::<Unread>::default(),
            mentions,
        })
    }

    /// The mark these counts give the channel. A mention in a thread marks
    /// it with no unread message in its segments.
    fn activity(self, channel_id: Id<Channel>) -> ChannelActivity {
        ChannelActivity {
            channel_id,
            active: ambre_bus::Activity::from_bool(
                !self.unread.is_zero() || self.mentions != ambre_core::Count::<Mentions>::ZERO,
            ),
            unread: Some(self.unread),
            mentions: Some(self.mentions),
        }
    }
}

/// A command that writes a channel or a space document, held until that
/// document may be written (`AUTHOR-026`).
#[derive(Debug)]
pub enum HeldCommand {
    /// A command of an open channel.
    Channel(Id<Channel>, MurmureCommand),
    /// A command of an open space.
    Space(Id<Space>, crate::message::SpaceCommand),
    /// An invite into a direct message, held until its notice can be
    /// written (the Murmure channels spec).
    Invite(Seq<Request>, ambre_bus::RestCommand),
}

/// The counts of an open channel, `None` when a part of them is unknown
/// (the Murmure unread spec). Unknown is a held segment with no cursor yet,
/// or a final segment not loaded that may hold unread messages. The
/// mentions are read off the marks, never off a summary (the Murmure
/// mentions spec).
fn counts_of(
    channel: &ChannelState,
    cursors: &HashMap<Id<Document>, ReadCursor>,
    me: Id<User>,
) -> Option<Counts> {
    let held = channel.held_segments();
    let mut count = Counts::default();
    for segment in &held {
        let cursor = cursors.get(&segment.document_id())?;
        let counted = channel.counts(*segment, cursor.read, me).ok()?;
        count = count.plus(&Counts {
            unread: counted.unread,
            mentions: counted.mentions,
        });
    }
    // A held thread counts its mentions alone, from its cursor or from its
    // start (the Murmure threads spec).
    for thread in channel.held_threads() {
        let read = cursors
            .get(&thread)
            .map_or(Seq::<Relay>::ZERO, |cursor| cursor.read);
        count = count.with_mentions(channel.thread_mentions(thread, read, me).ok()?);
    }
    let unknown_final = channel
        .final_segments()
        .into_iter()
        .filter(|segment| !held.contains(segment))
        .any(|segment| {
            cursors
                .get(&segment.document_id())
                .is_none_or(|c| c.active == ambre_bus::Activity::Active)
        });
    (!unknown_final).then_some(count)
}

impl<D> Runtime<D> {
    /// The mark of an open channel, counted, or `None` when its count is
    /// not known yet.
    pub fn counted(&self, channel_id: Id<Channel>) -> Option<ChannelActivity> {
        let channel = self.channels.get(&channel_id)?;
        let counts = counts_of(channel, &self.read_cursors, self.my_user_id?)?;
        Some(
            counts
                .with_mentions(self.thread_mentions(channel_id))
                .activity(channel_id),
        )
    }

    /// Hand the view the count of an open channel, once it is known.
    fn emit_count(&self, channel_id: Id<Channel>) {
        if let Some(mark) = self.counted(channel_id) {
            let unread = mark.unread;
            self.emit(UiEvent::ChannelActivity {
                channels: vec![mark],
            });
            self.emit_counted_first_unread(channel_id, unread);
        }
    }

    /// Hand the view the first unread message of an open channel the caller
    /// just counted at `unread`. The line of the message list sits above it.
    /// A count of zero names no first message, and walks nothing.
    fn emit_counted_first_unread(
        &self,
        channel_id: Id<Channel>,
        unread: Option<ambre_core::Count<Unread>>,
    ) {
        let (Some(channel), Some(me)) = (self.channels.get(&channel_id), self.my_user_id) else {
            return;
        };
        if unread.is_some_and(|count| count.is_zero()) {
            let event = MurmureEvent::FirstUnread { message: None };
            return self.emit(UiEvent::Channel { channel_id, event });
        }
        let read_of = |segment: Id<Segment>| {
            self.read_cursors
                .get(&segment.document_id())
                .map_or(Seq::<Relay>::ZERO, |cursor| cursor.read)
        };
        let message = channel.first_unread(read_of, me);
        let event = MurmureEvent::FirstUnread { message };
        self.emit(UiEvent::Channel { channel_id, event });
    }

    /// Ask the relay for the documents of a channel (the Murmure engine spec).
    /// A channel already open or opening is left as it is.
    pub fn open_channel(&mut self, channel_id: Id<Channel>) {
        if self.channels.contains_key(&channel_id) || self.opening.contains_key(&channel_id) {
            return;
        }
        // A channel closed earlier keeps its light subscriptions while it
        // opens: each document reopens from its local copy (the Murmure
        // engine spec). Its read cursors stay where they were (`RESID-013`).
        self.opening.insert(channel_id, None);
        // A channel a catalogue lists is public: its log admits a member who
        // joined alone, and heeds the log of its space. Any other channel's
        // log admits only invites (`SYNC-106`).
        // Its catalogue entry names the channel key every open trusts
        // (`SYNC-299`).
        let listing = self.listing(channel_id);
        self.ask(SyncCommand::OpenChannel {
            channel_id,
            listed_in: listing.as_ref().map(|(space_id, _)| *space_id),
            catalogue_key: listing.map(|(_, key)| key),
        });
    }

    /// Hold a channel the relay just created, and open the session of its two
    /// documents. Each is written again under its PeerID once its identity
    /// lands. Its whole content leaves as its first update once the Sync
    /// worker holds the document.
    pub fn hold_created(&mut self, channel: ChannelState) {
        let meta = channel.meta_id();
        for document_id in [meta.document_id(), channel.active_id().document_id()] {
            self.ask(open_in_channel(meta, document_id));
        }
        self.channels.insert(channel.channel_id(), channel);
    }

    /// Note the session identity Sync issued for a channel or a space
    /// document. An open channel or space holding it adopts it (`AUTHOR-002`). An identity no channel or space expects is
    /// stale, and dropped.
    pub fn adopt_murmure_session(
        &mut self,
        document_id: Id<Document>,
        peer: ambre_core::PeerId,
        entry: ambre_core::PeerEntry,
    ) {
        let space = Id::<Space>::from_document(document_id);
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
    pub(crate) fn settle_murmure_session(&mut self, document_id: Id<Document>) {
        let Some((peer, entry)) = self.murmure_identities.get(&document_id).cloned() else {
            return;
        };
        let adopted =
            if let Some(channel) = self.channels.values_mut().find(|c| c.holds(document_id)) {
                channel.adopt_session(
                    document_id,
                    peer,
                    ambre_core::Payload::<ambre_core::PeerEntryBytes>::from(&entry),
                )
            } else if let Some(adopted) = self.adopt_space_session(document_id, peer, &entry) {
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
    pub(crate) fn murmure_writable(&self, document_id: Id<Document>, identified: bool) -> bool {
        identified && !self.unsent.contains_key(&document_id)
    }

    /// Hold a command until `document_id` may be written. Past the cap the
    /// command is dropped and logged, so the ones held keep their order.
    pub(crate) fn wait_for_session(&mut self, document_id: Id<Document>, command: HeldCommand) {
        let waiting = self.murmure_waiting.entry(document_id).or_default();
        if waiting.len() >= HELD_EDITS_LIMIT {
            tracing::warn!("Logic: too many edits wait on {document_id}; dropped");
            return;
        }
        waiting.push(command);
    }

    /// Run, in their order, the commands that waited on `document_id`. One
    /// that still cannot write waits again.
    pub(crate) fn release_waiting(&mut self, document_id: Id<Document>) {
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
                HeldCommand::Invite(id, cmd) => {
                    self.forward_invite(id, cmd);
                }
            }
        }
    }

    /// Forget what waited on a document whose session ended. An invite held
    /// for its notice is refused, so the view does not wait for it.
    pub fn forget_murmure_session(&mut self, document_id: Id<Document>) {
        self.murmure_identities.remove(&document_id);
        self.unnamed_roots.remove(&document_id);
        self.unsent.remove(&document_id);
        for held in self
            .murmure_waiting
            .remove(&document_id)
            .unwrap_or_default()
        {
            if let HeldCommand::Invite(id, _) = held {
                self.refuse_invite(id, ambre_core::LocalFailure::ChannelNotOpen);
            }
        }
    }

    /// The view of a channel closed. Each document of an open channel reaches
    /// the local store, then leaves memory as a light subscription. Its
    /// summaries count the channel's new messages (`RESID-004`, `RESID-006`).
    /// A channel still opening ends the session
    /// of each document it asked for.
    pub fn close_channel(&mut self, channel_id: Id<Channel>) {
        let Some(channel) = self.channels.remove(&channel_id) else {
            return self.end_channel(channel_id);
        };
        self.meta_roles.remove(&channel.meta_id().document_id());
        // Its documents leave the relay's watch, so the other members see
        // this user go (`SRV-097`). A reopen lists the members again.
        self.channel_watchers.remove(&channel_id);
        self.closing_segments.remove(&channel_id);
        self.close_successors.remove(&channel_id);
        self.drop_replays(channel_id);
        self.channel_members.remove(&channel_id);
        let base = self
            .my_user_id
            .and_then(|me| counts_of(&channel, &self.read_cursors, me));
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
                .map_or(Seq::<Relay>::ZERO, |ledger| ledger.contiguous_end());
            self.forget_murmure_session(document_id);
            // The copy carries the relay's numbers it knows, so a reopen
            // counts from it. One that fails to export is fetched whole.
            let applied = match channel.snapshot(document_id) {
                Ok(copy) => {
                    self.store(document_id, StoreWrite::Snapshot(copy));
                    self.store(document_id, StoreWrite::Hidden);
                    copies.insert(document_id, horizon);
                    horizon
                }
                Err(error) => {
                    tracing::warn!("Logic: no local copy of {document_id}: {error}");
                    Seq::<Relay>::ZERO
                }
            };
            self.ask(SyncCommand::ReleaseDocument {
                document_id,
                applied_seq: applied,
            });
        }
        self.closed_channels.insert(
            channel_id,
            ClosedChannel {
                meta: channel.meta_id(),
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
    fn open_channel_document(
        &mut self,
        channel_id: Id<Channel>,
        meta: Id<Meta>,
        document_id: Id<Document>,
    ) {
        if let Some(closed) = self.closed_channels.get_mut(&channel_id)
            && let Some(horizon) = closed.copies.remove(&document_id)
        {
            closed.reading.insert(document_id, horizon);
            self.store(document_id, StoreWrite::ReadBack);
            return;
        }
        self.ask(open_in_channel(meta, document_id));
    }

    /// The local copy of a reopening channel's document landed. The opening
    /// takes it as a snapshot at its horizon, and the Sync worker replays
    /// only what follows (`RESID-010`). An empty copy drops the light
    /// subscription, so the open fetches the document whole (`RESID-012`).
    pub fn channel_copy_landed(
        &mut self,
        document_id: Id<Document>,
        snapshot: Option<ambre_core::Payload<ambre_core::SnapshotBytes>>,
        deltas: &[ambre_core::Payload<ambre_core::DeltaBytes>],
    ) {
        let Some((&channel_id, closed)) = self
            .closed_channels
            .iter_mut()
            .find(|(_, closed)| closed.reading.contains_key(&document_id))
        else {
            return;
        };
        let horizon = closed
            .reading
            .remove(&document_id)
            .unwrap_or(Seq::<Relay>::ZERO);
        // An opening that ended meanwhile closed the subscription already.
        let Some(opening) = self
            .opening
            .get_mut(&channel_id)
            .and_then(Option::as_mut)
            .filter(|opening| opening.expects(document_id))
        else {
            return;
        };
        let meta = opening.meta_id();
        if let Some(copy) = snapshot {
            trace_receipt(
                document_id,
                opening.receive_at(document_id, Some(Order::Snapshot(horizon)), (&copy).into()),
            );
            for delta in deltas {
                trace_receipt(document_id, opening.receive(document_id, delta.into()));
            }
            self.channel_ledgers
                .entry(document_id)
                .or_default()
                .raise_floor(horizon);
        } else {
            tracing::warn!("Logic: no usable local copy of {document_id}, fetching it");
            self.ask(SyncCommand::CloseDocument { document_id });
        }
        self.ask(open_in_channel(meta, document_id));
    }

    /// Whether a reopening channel is reading back the copy of `document_id`.
    pub fn reads_channel_copy(&self, document_id: Id<Document>) -> bool {
        self.closed_channels
            .values()
            .any(|closed| closed.reading.contains_key(&document_id))
    }

    /// A reopened channel is open: the record of its close goes, and the
    /// light subscription of each document it no longer holds ends.
    fn settle_reopen(&mut self, channel_id: Id<Channel>) {
        let Some(closed) = self.closed_channels.remove(&channel_id) else {
            return;
        };
        let Some(channel) = self.channels.get(&channel_id) else {
            return;
        };
        let left: Vec<Id<Document>> = closed
            .documents
            .into_iter()
            .filter(|id| !channel.holds(*id))
            .collect();
        for document_id in left {
            self.ask(SyncCommand::CloseDocument { document_id });
        }
    }

    /// End a channel, open, opening or closed: the session or the light
    /// subscription of each of its documents ends, so Sync forgets its key.
    pub fn end_channel(&mut self, channel_id: Id<Channel>) {
        self.end_light_channel(channel_id);
        self.end_watched_threads(channel_id);
        self.channel_watchers.remove(&channel_id);
        self.closing_segments.remove(&channel_id);
        self.close_successors.remove(&channel_id);
        self.drop_replays(channel_id);
        self.channel_members.remove(&channel_id);
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
            self.meta_roles.remove(&document_id);
            self.forget_murmure_session(document_id);
            self.ask(SyncCommand::CloseDocument { document_id });
        }
    }

    /// End the light subscriptions a closed channel left, if it left any.
    pub(crate) fn end_light_channel(&mut self, channel_id: Id<Channel>) {
        let Some(closed) = self.closed_channels.remove(&channel_id) else {
            return;
        };
        for document_id in closed.documents {
            self.ask(SyncCommand::CloseDocument { document_id });
        }
    }

    /// Count the activity of a closed channel's document for its channel,
    /// and hand the view the channel's new mark (the Murmure unread spec).
    /// Returns whether a closed channel holds the document.
    pub fn count_closed(
        &mut self,
        document_id: Id<Document>,
        counters: ambre_core::activity::ActivityCounters,
    ) -> bool {
        let Some((&channel_id, closed)) = self
            .closed_channels
            .iter_mut()
            .find(|(_, closed)| closed.holds(document_id))
        else {
            return false;
        };
        let now = Counts {
            unread: ambre_core::Count::<Unread>::recount(counters.content),
            mentions: ambre_core::Count::<Mentions>::recount(counters.mentions),
        };
        let before = closed.counted.insert(document_id, now).unwrap_or_default();
        // The summaries counted a message since the last count: that is
        // news (the Murmure notifications spec).
        let news = (now.unread > before.unread).then_some(ambre_bus::MentionState::from_bool(
            now.mentions > before.mentions,
        ));
        if let Some(mark) = closed.mark(
            channel_id,
            threads::mentions_of(&self.watched_threads, channel_id),
        ) {
            self.emit(UiEvent::ChannelActivity {
                channels: vec![mark],
            });
        }
        if let Some(mention) = news {
            self.emit(UiEvent::NewMessage {
                channel_id,
                mention,
                message: None,
            });
        }
        true
    }

    /// The relay listed the documents of a channel: open its `meta` first,
    /// which names the live segments (the Murmure engine spec).
    pub fn channel_listed(&mut self, channel_id: Id<Channel>, documents: &[ChannelDocument]) {
        // A closed channel was listed again for a segment opened since.
        if !self.opening.contains_key(&channel_id) {
            return self.watch_listed(channel_id, documents);
        }
        // A listing for a channel listed twice is stale.
        let Some(slot @ None) = self.opening.get_mut(&channel_id) else {
            return;
        };
        let Some(meta) = documents
            .iter()
            .find(|d| d.kind == ChannelDocumentKind::Meta)
        else {
            self.opening.remove(&channel_id);
            self.channel_open_failed(channel_id, &"the channel has no meta document");
            return;
        };
        let meta = Id::<Meta>::from_document(meta.document_id);
        *slot = Some(OpeningChannel::new(channel_id, meta));
        self.open_channel_document(channel_id, meta, meta.document_id());
    }

    /// Tell the view a channel could not open. Nothing of it is held.
    pub(crate) fn channel_open_failed(
        &self,
        channel_id: Id<Channel>,
        cause: &dyn std::fmt::Display,
    ) {
        tracing::warn!("Logic: channel {channel_id} failed to open: {cause}");
        self.emit(UiEvent::Channel {
            channel_id,
            event: MurmureEvent::OpenFailed,
        });
    }

    /// The relay refused the listing for want of membership. The user left
    /// the channel, or it went, by a request whose answer never came back.
    /// Nothing of it was held yet, so the view alone learns it is gone.
    pub fn channel_refused(&mut self, channel_id: Id<Channel>) {
        if self.opening.remove(&channel_id).is_none() {
            return;
        }
        self.end_light_channel(channel_id);
        tracing::info!("Logic: channel {channel_id} refused its open");
        self.emit(UiEvent::Channel {
            channel_id,
            event: MurmureEvent::Left,
        });
    }

    /// A document of a channel has all its bytes, and `role` is this
    /// client's role on it. Returns `false` when no channel, open or
    /// opening, asked for it.
    pub fn channel_document_ready(&mut self, document_id: Id<Document>, role: Role) -> bool {
        self.note_meta_role(document_id, role);
        if let Some((&channel_id, channel)) =
            self.channels.iter_mut().find(|(_, c)| c.holds(document_id))
        {
            // A probe is read for the segment its header names, then
            // closed: nothing holds it.
            if channel.is_probe(document_id) {
                if let Err(error) = channel.probe_ready(document_id) {
                    tracing::warn!("Logic: thread {document_id} names no root: {error}");
                }
                self.forget_murmure_session(document_id);
                self.ask(SyncCommand::CloseDocument { document_id });
                self.advance_seek(channel_id);
                return true;
            }
            // An older segment the view asked for. A second
            // ready for a live document, after a reconnect, changes nothing.
            if channel.is_pending(document_id) {
                match channel.older_ready(document_id) {
                    Ok(event) => {
                        self.emit(UiEvent::Channel { channel_id, event });
                        self.show_thread(channel_id, document_id);
                        self.emit_count(channel_id);
                        // An amendments document or a thread takes the
                        // changes that waited on it, and a loaded segment
                        // may have some.
                        self.release_waiting(document_id);
                        self.open_amendments(channel_id);
                        self.open_threads(channel_id);
                    }
                    Err(error) => {
                        // Its key opened it, so the segment itself is
                        // damaged, never a missing key.
                        let band = channel.band(document_id, Unreadable::Damaged);
                        tracing::warn!("Logic: older segment {document_id} refused: {error}");
                        self.ask(SyncCommand::CloseDocument { document_id });
                        if let Some(event) = band {
                            self.emit(UiEvent::Channel { channel_id, event });
                        }
                    }
                }
                self.advance_seek(channel_id);
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
        let asked = opening.document_ids();
        let result = opening.ready(document_id);
        self.advance_opening(channel_id, asked, result);
        true
    }

    /// Keep `role` when `document_id` is the `meta` of a channel open or
    /// opening here. A role that changes on an open channel may let this
    /// client seal its name now (`MURMURE-684`).
    fn note_meta_role(&mut self, document_id: Id<Document>, role: Role) {
        let open = self
            .channels
            .iter()
            .find(|(_, c)| c.meta_id().document_id() == document_id)
            .map(|(id, _)| *id);
        let opening = self
            .opening
            .values()
            .flatten()
            .any(|o| o.meta_id().document_id() == document_id);
        if open.is_none() && !opening {
            return;
        }
        self.meta_roles.insert(document_id, role);
        if let Some(channel_id) = open {
            self.reseal_stale_name(channel_id);
        }
    }

    /// Act on where an opening stands. `asked` lists every document it asked
    /// for: those the open channel does not hold are closed.
    fn advance_opening(
        &mut self,
        channel_id: Id<Channel>,
        asked: Vec<Id<Document>>,
        result: Result<Opening, MurmureError>,
    ) {
        match result {
            Ok(Opening::Waiting) => {}
            Ok(Opening::OpenSegments(segments)) => {
                // The live segments the sealed `meta` named.
                let Some(meta) = self
                    .opening
                    .get(&channel_id)
                    .and_then(Option::as_ref)
                    .map(OpeningChannel::meta_id)
                else {
                    return;
                };
                for segment in segments {
                    self.open_channel_document(channel_id, meta, segment.document_id());
                }
            }
            Ok(Opening::Open(mut state)) => {
                self.opening.remove(&channel_id);
                self.emit(UiEvent::Channel {
                    channel_id,
                    event: MurmureEvent::Named { name: state.name() },
                });
                // A closing segment the channel opened without shows as a
                // band, and its session ends.
                if let Some(event) = state.take_closing_band() {
                    self.emit(UiEvent::Channel { channel_id, event });
                }
                let held = state.document_ids();
                for document_id in asked.into_iter().filter(|id| !held.contains(id)) {
                    self.forget_murmure_session(document_id);
                    self.ask(SyncCommand::CloseDocument { document_id });
                }
                // The closing segment first, so the view draws the history in
                // order.
                for segment in [state.closing_id(), Some(state.active_id())]
                    .into_iter()
                    .flatten()
                {
                    match state.messages(segment) {
                        Ok(messages) => self.emit(UiEvent::Channel {
                            channel_id,
                            event: MurmureEvent::Messages { segment, messages },
                        }),
                        Err(error) => tracing::warn!("Logic: segment {segment}: {error}"),
                    }
                }
                let documents = state.document_ids();
                self.channels.insert(channel_id, *state);
                self.reseal_stale_name(channel_id);
                self.show_channel_members(channel_id);
                self.settle_reopen(channel_id);
                // Each identity landed while the channel was opening.
                for document_id in documents {
                    self.settle_murmure_session(document_id);
                }
                self.open_amendments(channel_id);
                self.open_threads(channel_id);
                self.emit_count(channel_id);
                for decline in self.declines.remove(&channel_id).unwrap_or_default() {
                    self.handle_channel_command(channel_id, decline);
                }
            }
            Err(error) => self.end_opening(channel_id, &error),
        }
    }

    /// Move the seek for the root of a copy one step (the Murmure threads
    /// spec). It opens the next older segment, or tells the view the root.
    fn advance_seek(&mut self, channel_id: Id<Channel>) {
        let Some((meta, step)) = self
            .channels
            .get_mut(&channel_id)
            .and_then(|channel| Some((channel.meta_id(), channel.seek_step()?)))
        else {
            return;
        };
        match step {
            SeekStep::Found {
                thread,
                segment,
                root,
            } => self.emit(UiEvent::Channel {
                channel_id,
                event: MurmureEvent::RootFound {
                    thread,
                    segment,
                    root,
                },
            }),
            SeekStep::Load(segment) => {
                self.ask(open_in_channel(meta, segment.document_id()));
            }
            SeekStep::Wait => {}
            SeekStep::Lost { thread } => {
                tracing::warn!("Logic: channel {channel_id} holds no root for the copy");
                self.emit(UiEvent::Channel {
                    channel_id,
                    event: MurmureEvent::RootLost { thread },
                });
            }
        }
    }

    /// A document of a channel could not open. Returns `false` when no
    /// channel asked for it.
    pub fn channel_document_failed(
        &mut self,
        document_id: Id<Document>,
        cause: OpenFailure,
    ) -> bool {
        if cause == OpenFailure::RootOutsideSpace {
            self.end_rooted_outside(document_id);
            return true;
        }
        if let Some((&channel_id, channel)) =
            self.channels.iter_mut().find(|(_, c)| c.holds(document_id))
        {
            // A missing key earns a band in the list. A transient failure
            // earns none: asking again may open it. A refusal earns none
            // either: the band names a key, and the relay's answer is told.
            // Nor does a key no member signed: the band names a wait, and the
            // relay's fault is no wait (`SYNC-212`).
            let band = match cause {
                OpenFailure::KeyNotReceived => {
                    channel.band(document_id, Unreadable::KeyNotReceived)
                }
                OpenFailure::Transient
                | OpenFailure::Refused { .. }
                | OpenFailure::KeyUnsigned
                | OpenFailure::RootOutsideSpace => None,
            };
            // An older segment is given up, so the view can ask again. A live
            // document of an open channel keeps what it holds.
            channel.cancel_older(document_id);
            tracing::warn!("Logic: channel document {document_id} failed to open: {cause}");
            if let Some(event) = band {
                self.emit(UiEvent::Channel { channel_id, event });
            }
            self.advance_seek(channel_id);
            return true;
        }
        let Some(channel_id) = self.opening.iter().find_map(|(id, o)| {
            o.as_ref()
                .is_some_and(|o| o.expects(document_id))
                .then_some(*id)
        }) else {
            return false;
        };
        // A closing segment whose key never came is given up, and the channel
        // opens without it. Any other failure ends the opening.
        let given_up = match (cause, self.opening.get_mut(&channel_id)) {
            (OpenFailure::KeyNotReceived, Some(Some(opening))) => {
                let asked = opening.document_ids();
                opening.give_up(document_id).map(|result| (asked, result))
            }
            _ => None,
        };
        let Some((asked, result)) = given_up else {
            self.end_opening(channel_id, &cause);
            return true;
        };
        tracing::warn!("Logic: closing segment {document_id} given up: {cause}");
        self.forget_murmure_session(document_id);
        self.ask(SyncCommand::CloseDocument { document_id });
        let asked = asked.into_iter().filter(|id| *id != document_id).collect();
        self.advance_opening(channel_id, asked, result);
        true
    }

    /// The log of the channel that holds `document_id` starts at no writer
    /// of its space (audit `N16`). Every document of the channel walks that
    /// log, so the whole channel ends, open, opening or closed. Each message
    /// and key it held leaves memory, and the view drops the channel as one
    /// that failed to open (`SYNC-231`). A document no channel holds any more
    /// belongs to a channel an earlier failure already ended.
    fn end_rooted_outside(&mut self, document_id: Id<Document>) {
        let Some(channel_id) = self.channel_anywhere(document_id) else {
            tracing::warn!(
                "Logic: {document_id} starts outside its space, and no channel holds it"
            );
            self.ask(SyncCommand::CloseDocument { document_id });
            self.store(document_id, StoreWrite::Erase);
            return;
        };
        let copies = self.documents_anywhere(channel_id);
        self.end_channel(channel_id);
        self.channel_open_failed(channel_id, &OpenFailure::RootOutsideSpace);
        // No copy of what the forged log vouched for stays in the store, so
        // no later open shows it (`SYNC-233`).
        for document_id in copies {
            self.store(document_id, StoreWrite::Erase);
        }
    }

    /// The channel that holds `document_id`, whether it is open, opening or
    /// closed.
    fn channel_anywhere(&self, document_id: Id<Document>) -> Option<Id<Channel>> {
        self.channels
            .iter()
            .find(|(_, c)| c.holds(document_id))
            .map(|(id, _)| *id)
            .or_else(|| {
                self.opening.iter().find_map(|(id, o)| {
                    o.as_ref()
                        .is_some_and(|o| o.expects(document_id))
                        .then_some(*id)
                })
            })
            .or_else(|| {
                self.closed_channels
                    .iter()
                    .find(|(_, c)| c.documents.contains(&document_id))
                    .map(|(id, _)| *id)
            })
    }

    /// A revoke capped a writer below a delta `document_id` applied, so the
    /// state of its channel holds a write the log refuses (`SYNC-300`). A
    /// merge cannot take that write out. The whole channel ends, and each
    /// local copy of its documents leaves the store. The channel opens again
    /// from the relay, which replays each document under the cap. Returns
    /// `false` when no channel holds the document.
    pub fn channel_reread(&mut self, document_id: Id<Document>) -> bool {
        let Some(channel_id) = self.channel_anywhere(document_id) else {
            return false;
        };
        let copies = self.documents_anywhere(channel_id);
        self.end_channel(channel_id);
        for document_id in copies {
            self.store(document_id, StoreWrite::Erase);
        }
        self.open_channel(channel_id);
        true
    }

    /// Every document of `channel_id` this thread knows, whether the channel
    /// is open, opening or closed.
    fn documents_anywhere(&self, channel_id: Id<Channel>) -> HashSet<Id<Document>> {
        let open = self
            .channels
            .get(&channel_id)
            .map(ChannelState::document_ids);
        let opening = self
            .opening
            .get(&channel_id)
            .and_then(Option::as_ref)
            .map(OpeningChannel::document_ids);
        let closed = self
            .closed_channels
            .get(&channel_id)
            .map(|closed| closed.documents.clone());
        [open, opening, closed]
            .into_iter()
            .flatten()
            .flatten()
            .collect()
    }

    /// Drop an opening that failed, end the session of each document it asked
    /// for, and tell the view.
    fn end_opening(&mut self, channel_id: Id<Channel>, cause: &dyn std::fmt::Display) {
        if let Some(Some(opening)) = self.opening.remove(&channel_id) {
            for document_id in opening.document_ids() {
                self.forget_murmure_session(document_id);
                self.ask(SyncCommand::CloseDocument { document_id });
            }
        }
        self.end_light_channel(channel_id);
        // A decline that waited on the open writes nothing: the ring ends at its timeout.
        self.declines.remove(&channel_id);
        self.channel_open_failed(channel_id, cause);
    }

    /// The snapshot of a channel document, for the relay's compaction, with
    /// the last delta it folds in (`SRV-083`).
    pub fn channel_snapshot(
        &self,
        document_id: Id<Document>,
    ) -> Option<(ambre_core::Payload<ambre_core::SnapshotBytes>, Seq<Relay>)> {
        let channel = self.channels.values().find(|c| c.holds(document_id))?;
        let payload = channel
            .snapshot(document_id)
            .map_err(|e| tracing::warn!("Logic: snapshot of {document_id} failed: {e}"))
            .ok()?;
        let covered = self
            .channel_ledgers
            .get(&document_id)
            .map_or(Seq::<Relay>::ZERO, SeqLedger::contiguous_end);
        Some((payload, covered))
    }

    /// Check a snapshot of a channel document against the state this client
    /// holds, and answer the verdict to Sync (the Murmure segments spec). A
    /// document no open channel holds is not answered.
    pub fn check_channel_snapshot(
        &mut self,
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        covered: Seq<Relay>,
        payload: &ambre_core::Payload<ambre_core::SnapshotBytes>,
        purpose: ambre_bus::SnapshotPurpose,
    ) {
        let Some(channel) = self.channels.values().find(|c| c.holds(document_id)) else {
            return;
        };
        let ledger = self.channel_ledgers.get(&document_id);
        let floor = ledger.map_or(Seq::<Relay>::ZERO, SeqLedger::floor);
        let end = ledger.map_or(Seq::<Relay>::ZERO, SeqLedger::contiguous_end);
        let confirm = purpose == ambre_bus::SnapshotPurpose::Confirm;
        let verdict = if covered < floor || (confirm && end < covered) {
            SnapshotVerdict::Unchecked
        } else {
            channel
                .check_snapshot(document_id, covered, payload)
                .unwrap_or_else(|e| {
                    tracing::warn!("Logic: check of {document_id} failed: {e}");
                    SnapshotVerdict::Unchecked
                })
        };
        if confirm
            && matches!(verdict, SnapshotVerdict::Match { .. })
            && let Some(ledger) = self.channel_ledgers.get_mut(&document_id)
        {
            ledger.fold_through(covered);
        }
        self.ask(SyncCommand::SnapshotVerdict {
            document_id,
            snapshot_id,
            purpose,
            verdict,
        });
    }

    /// Note the relay's number of a delta for a channel or a space document,
    /// open or opening. A document neither holds is not noted.
    pub fn record_channel_seq(
        &mut self,
        document_id: Id<Document>,
        seq: Seq<Relay>,
        payload: Option<&ambre_core::Payload<ambre_core::DeltaBytes>>,
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
    pub fn note_murmure_snapshot(&mut self, document_id: Id<Document>, covered: Seq<Relay>) {
        if self.holds_murmure(document_id) {
            self.channel_ledgers
                .entry(document_id)
                .or_default()
                .raise_floor(covered);
        }
    }

    /// Whether an open or opening channel or space holds `document_id`.
    fn holds_murmure(&self, document_id: Id<Document>) -> bool {
        let space = Id::<Space>::from_document(document_id);
        self.channels.values().any(|c| c.holds(document_id))
            || self
                .opening
                .values()
                .flatten()
                .any(|o| o.expects(document_id))
            || self.spaces.contains_key(&space)
            || self.space_opening.contains_key(&space)
    }

    /// The question an unconfirmed `@channel` past the threshold asks, with
    /// the send it holds. `None` lets the command run.
    fn held_send(&self, channel_id: Id<Channel>, command: &MurmureCommand) -> Option<MurmureEvent> {
        let (draft, attachments, reply) = match command {
            MurmureCommand::SendMessage {
                draft,
                attachments,
                confirmed: crate::message::Confirmation::Unconfirmed,
                ..
            } => (draft, attachments.clone(), None),
            MurmureCommand::Reply {
                root,
                draft,
                also_in_channel,
                confirmed: crate::message::Confirmation::Unconfirmed,
                ..
            } => {
                let reply = HeldReply {
                    root: *root,
                    also_in_channel: *also_in_channel,
                };
                (draft, Vec::new(), Some(reply))
            }
            _ => return None,
        };
        let all = |mention: &Mention| mention.target == MentionTarget::Channel;
        if !draft.mentions.iter().any(all) {
            return None;
        }
        let members: Vec<Id<User>> = self
            .channel_members
            .get(&channel_id)?
            .iter()
            .map(|row| row.user_id)
            .collect();
        match screen(
            draft.clone(),
            Some(&members),
            crate::message::Confirmation::Unconfirmed,
        ) {
            Screened::Send(_) => None,
            Screened::Confirm { members, draft } => Some(MurmureEvent::ConfirmChannelMention {
                members,
                held: HeldSend {
                    draft,
                    attachments,
                    reply,
                },
            }),
        }
    }

    /// Run one command on an open channel, and emit what it produced
    /// (`BUS-027`).
    ///
    /// The delta leaves for the Sync worker under the document it names, with
    /// its activity summary (the Murmure messages spec). The event goes to the view.
    pub fn handle_channel_command(&mut self, channel_id: Id<Channel>, command: MurmureCommand) {
        // An `@channel` past the threshold is asked about before anything
        // moves, so a cancel leaves no thread created for it.
        if let Some(event) = self.held_send(channel_id, &command) {
            return self.emit(UiEvent::Channel { channel_id, event });
        }
        // A ring sounds on a page that may not hold its channel. The decline opens the
        // channel, and writes its end once it is open (the Murmure calls spec, the ring).
        if matches!(command, MurmureCommand::DeclineCall { .. })
            && !self.channels.contains_key(&channel_id)
        {
            self.declines.entry(channel_id).or_default().push(command);
            return self.open_channel(channel_id);
        }
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            tracing::warn!("Logic: command for channel {channel_id}, which is not open; dropped");
            return;
        };
        // A write before its document may be written waits: under any other
        // PeerID it would never travel (`AUTHOR-026`, `AUTHOR-029`).
        let target = match &command {
            MurmureCommand::SendMessage { .. } | MurmureCommand::PostCall { .. } => {
                Some(channel.active_id().document_id())
            }
            // The end writes in the segment that holds the call message, whose session the
            // write finds or refuses (the Murmure calls spec).
            MurmureCommand::EndCall { .. } => None,
            // A decline may reach a channel that opened for it. It waits on the identity of
            // the segment of the call message (the Murmure calls spec, the ring).
            MurmureCommand::DeclineCall { call, .. } => {
                channel.call_segment(*call).map(|segment| segment.document_id())
            }
            MurmureCommand::Name { .. } => Some(channel.meta_id().document_id()),
            // A reply changes in its thread, which the channel holds once
            // its root is on screen (the Murmure threads spec).
            MurmureCommand::React {
                at: MessageAt::Thread(root),
                ..
            }
            | MurmureCommand::Unreact {
                at: MessageAt::Thread(root),
                ..
            }
            | MurmureCommand::EditMessage {
                at: MessageAt::Thread(root),
                ..
            }
            | MurmureCommand::DeleteMessage {
                at: MessageAt::Thread(root),
                ..
            } => {
                let Some(thread) = channel.thread_id(*root) else {
                    tracing::warn!("Logic: change in the thread of {root}, not held; refused");
                    return;
                };
                Some(thread)
            }
            MurmureCommand::React {
                at: MessageAt::Segment(segment),
                ..
            }
            | MurmureCommand::Unreact {
                at: MessageAt::Segment(segment),
                ..
            } => Some(segment.document_id()),
            // A locked segment takes the change in its amendments document,
            // which may first have to arrive or to be created
            // (the Murmure messages spec).
            MurmureCommand::EditMessage {
                at: MessageAt::Segment(segment),
                now,
                ..
            }
            | MurmureCommand::DeleteMessage {
                at: MessageAt::Segment(segment),
                now,
                ..
            } => {
                let (segment, now) = (*segment, *now);
                let (id, ask) = match change_ask(channel, channel_id, segment, now) {
                    Ok(target) => target,
                    Err(error) => {
                        tracing::warn!("Logic: change on {segment} refused: {error}");
                        return;
                    }
                };
                if let Some(ask) = ask {
                    self.ask(ask);
                }
                Some(id)
            }
            // A reply writes in the thread of its root, which may first have
            // to arrive or to be created (the Murmure threads spec).
            MurmureCommand::Reply { root, now, .. } => {
                let (root, now) = (*root, *now);
                let meta = channel.meta_id();
                // A target is found only for a root a held segment holds.
                let segment = channel
                    .root_segment(root)
                    .unwrap_or_else(|| channel.active_id());
                let (id, ask) = match channel.reply_target(root, now) {
                    Ok(ThreadTarget::Write(id)) => (id, None),
                    Ok(ThreadTarget::Open(id)) => (
                        id,
                        Some(open_in_channel(meta, id)),
                    ),
                    Ok(ThreadTarget::Create(id)) => (
                        id,
                        Some(SyncCommand::CreateThread {
                            channel_id,
                            meta,
                            segment,
                            root,
                            thread: id,
                        }),
                    ),
                    // A locked root's amendments record its thread, so the
                    // reply waits for them first (the Murmure threads spec).
                    Err(MurmureError::NoAmendments(segment)) => {
                        match change_ask(channel, channel_id, segment, now) {
                            Ok(target) => target,
                            Err(error) => {
                                tracing::warn!("Logic: reply to {root} refused: {error}");
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!("Logic: reply to {root} refused: {error}");
                        return;
                    }
                };
                if let Some(ask) = ask {
                    self.ask(ask);
                }
                Some(id)
            }
            MurmureCommand::LoadOlder { .. }
            | MurmureCommand::SeekRoot { .. }
            | MurmureCommand::MarkRead
            | MurmureCommand::MarkUnread { .. }
            | MurmureCommand::MarkThreadRead { .. }
            | MurmureCommand::Tick { .. }
            | MurmureCommand::Typing { .. }
            | MurmureCommand::Retry { .. }
            | MurmureCommand::FetchFile { .. }
            | MurmureCommand::ShowPicture { .. }
            // A file writes no document: its message does, once it sends.
            | MurmureCommand::UploadFile { .. } => None,
            // Murmure reads and writes no format yet, so neither half exists.
            MurmureCommand::Interop(interop) => match interop {
                ambre_bus::interop::InteropRequest::Import(never) => match *never {},
                ambre_bus::interop::InteropRequest::Export(never) => match *never {},
            },
        };
        if let Some(document_id) = target {
            let identified = self
                .channels
                .get(&channel_id)
                .is_some_and(|channel| channel.identified(document_id));
            if !self.murmure_writable(document_id, identified) {
                return self
                    .wait_for_session(document_id, HeldCommand::Channel(channel_id, command));
            }
        }
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let meta_id = channel.meta_id();
        match command {
            MurmureCommand::PostCall { call, now } => {
                let Some(author) = self.my_user_id else {
                    tracing::warn!("Logic: a call message before the session; dropped");
                    return;
                };
                let posted = channel.post_call(call, author, now).map(Some);
                self.send_change(channel_id, posted);
                self.close_if_full(channel_id, now);
            }
            MurmureCommand::EndCall { call, now } => {
                let Some(user) = self.my_user_id else {
                    tracing::warn!("Logic: the end of a call before the session; dropped");
                    return;
                };
                let ended = channel.end_call(call, user, now);
                self.send_change(channel_id, ended);
            }
            MurmureCommand::DeclineCall { call, now } => {
                let Some(user) = self.my_user_id else {
                    tracing::warn!("Logic: a decline before the session; dropped");
                    return;
                };
                let declined = channel.decline_call(call, user, now);
                self.send_change(channel_id, declined);
            }
            MurmureCommand::SendMessage {
                draft,
                attachments,
                now,
                ..
            } => {
                // A mention of anyone outside the channel is no mark (the
                // Murmure mentions spec). The `@channel` question was asked
                // on the way in.
                let members: Option<Vec<Id<User>>> = self
                    .channel_members
                    .get(&channel_id)
                    .map(|rows| rows.iter().map(|row| row.user_id).collect());
                let draft = members_only(draft, members.as_deref());
                // The author is this session's user. Before `SessionReady`
                // there is none, and a message with no author is refused
                // rather than written under a placeholder (the Murmure schema).
                let Some(author) = self.my_user_id else {
                    tracing::warn!(
                        "Logic: message for channel {channel_id} before the session; dropped"
                    );
                    return;
                };
                match channel.send_attached(&draft, &attachments, author, now) {
                    Ok(sent) => {
                        self.ask(SyncCommand::SendUpdate {
                            document_id: sent.segment.document_id(),
                            payload: sent.delta,
                            summary: Some(sent.summary),
                        });
                        self.emit(UiEvent::Channel {
                            channel_id,
                            event: sent.event,
                        });
                        self.close_if_full(channel_id, now);
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
                    self.ask(open_in_channel(meta_id, older.document_id()));
                }
            }
            MurmureCommand::SeekRoot { thread } => {
                // The thread's header names the segment of its root. A
                // probe reads it, then the older segments load down to it.
                if let Some(probe) = channel.seek_root(thread) {
                    self.ask(open_in_channel(meta_id, probe));
                }
                self.advance_seek(channel_id);
            }
            // A channel's files live under its meta, which outlives every
            // segment (the Murmure attachments spec). The answer reaches the
            // view under the request it named.
            MurmureCommand::UploadFile { id, data } => {
                let document_id = channel.meta_id().document_id();
                self.ask(SyncCommand::Rest {
                    id,
                    cmd: RestCommand::UploadMedia { document_id, data },
                });
            }
            MurmureCommand::FetchFile { id, media_ref } => {
                let document_id = channel.meta_id().document_id();
                self.ask(SyncCommand::Rest {
                    id,
                    cmd: RestCommand::DownloadMedia {
                        document_id,
                        content_hash: media_ref,
                    },
                });
            }
            // The image of a preview decodes on the Logic thread, off the
            // render thread (`ambre-spec-murmure-attachments.md`).
            MurmureCommand::ShowPicture { media_ref } => {
                let document_id = channel.meta_id().document_id();
                self.owe(Effect::Preview {
                    channel_id,
                    document_id,
                    media_ref,
                });
            }
            MurmureCommand::Retry { segment } => {
                // Sync walks the key chain again at each open, so a new open
                // serves a missing key and damaged bytes alike (the Murmure
                // engine spec).
                if let Some(segment) = channel.retry(segment) {
                    self.ask(open_in_channel(meta_id, segment.document_id()));
                } else if channel.lacks_closing(segment) {
                    // The closing segment has no place to load into: the
                    // channel opens anew, from its local copies.
                    self.close_channel(channel_id);
                    self.open_channel(channel_id);
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
                            active: ambre_bus::Activity::Quiet,
                        },
                    );
                }
                for segment in &finals {
                    let cursor =
                        self.read_cursors
                            .entry(segment.document_id())
                            .or_insert(ReadCursor {
                                read: Seq::<Relay>::ZERO,
                                active: ambre_bus::Activity::Quiet,
                            });
                    cursor.active = ambre_bus::Activity::Quiet;
                }
                self.ask(SyncCommand::MarkRead { live, finals });
                let counted = self.counted(channel_id);
                let known = counted.map(|mark| mark.unread);
                let mark = counted.unwrap_or(ChannelActivity {
                    channel_id,
                    active: ambre_bus::Activity::Quiet,
                    unread: None,
                    mentions: None,
                });
                self.emit(UiEvent::ChannelActivity {
                    channels: vec![mark],
                });
                if let Some(unread) = known {
                    self.emit_counted_first_unread(channel_id, unread);
                }
            }
            MurmureCommand::Name { name } => {
                // Trimmed once, so the sealed name is the name `meta` holds.
                let name = name
                    .as_ref()
                    .map(ambre_core::trimmed_name)
                    .filter(|n| !ambre_core::name_is_empty(n));
                match channel.name_direct(name.as_ref()) {
                    Ok(named) => {
                        self.ask(SyncCommand::SendUpdate {
                            document_id: named.meta.document_id(),
                            payload: named.delta,
                            summary: None,
                        });
                        // Sealed beside meta too, so the name shows before the
                        // channel opens (the Murmure layout spec).
                        self.note_sealed_name(channel_id, &name);
                        self.ask(SyncCommand::NameChannel {
                            meta: named.meta,
                            name,
                        });
                        self.emit(UiEvent::Channel {
                            channel_id,
                            event: named.event,
                        });
                    }
                    Err(error) => {
                        tracing::warn!("Logic: name of channel {channel_id} refused: {error}");
                        self.emit(UiEvent::ApiError {
                            id: None,
                            request: ambre_bus::ApiRequest::NameChannel,
                            failure: ambre_bus::ApiFailure::Local(
                                ambre_core::LocalFailure::NameRefused,
                            ),
                        });
                    }
                }
            }
            MurmureCommand::MarkUnread { segment, message } => {
                self.mark_unread(channel_id, segment, message);
            }
            MurmureCommand::MarkThreadRead { thread } => {
                // The view shows the thread, so the channel holds it. The
                // cursor takes what this client was delivered, as a live
                // segment's does: the count runs over, never under.
                if !channel.held_threads().contains(&thread) {
                    tracing::warn!("Logic: thread {thread} not held; its cursor stays");
                    return;
                }
                let read = channel.delivered(thread);
                self.read_cursors.insert(
                    thread,
                    ReadCursor {
                        read,
                        active: ambre_bus::Activity::Quiet,
                    },
                );
                self.ask(SyncCommand::MarkThreadRead { thread });
                self.emit_count(channel_id);
            }
            MurmureCommand::React {
                at,
                message,
                emoji,
                now,
            } => {
                let Some(user) = self.my_user_id else {
                    tracing::warn!("Logic: reaction on {channel_id} before the session; dropped");
                    return;
                };
                let reacted = channel.react(at, message, &emoji, user, now);
                self.send_change(channel_id, reacted);
            }
            MurmureCommand::Unreact { at, message, emoji } => {
                let Some(user) = self.my_user_id else {
                    tracing::warn!("Logic: reaction on {channel_id} before the session; dropped");
                    return;
                };
                let removed = channel.unreact(at, message, &emoji, user);
                self.send_change(channel_id, removed);
            }
            MurmureCommand::EditMessage {
                at,
                message,
                draft,
                now,
            } => {
                let Some(user) = self.my_user_id else {
                    tracing::warn!("Logic: edit on {channel_id} before the session; dropped");
                    return;
                };
                // A mention of anyone outside the channel is no mark. An edit
                // notifies no one, so an `@channel` asks for nothing.
                let members: Option<Vec<Id<User>>> = self
                    .channel_members
                    .get(&channel_id)
                    .map(|rows| rows.iter().map(|row| row.user_id).collect());
                let draft = members_only(draft, members.as_deref());
                let edited = channel.edit(at, message, &draft, user);
                let copy = match (&edited, at) {
                    (Ok(Some(_)), MessageAt::Thread(root)) => channel.copy_of(root, message),
                    _ => None,
                };
                self.send_change(channel_id, edited);
                // The copy of a reply takes its edit, through the path of a
                // message of its segment (the Murmure threads spec).
                if let Some((segment, copy)) = copy {
                    self.handle_channel_command(
                        channel_id,
                        MurmureCommand::EditMessage {
                            at: segment.into(),
                            message: copy,
                            draft,
                            now,
                        },
                    );
                }
            }
            MurmureCommand::DeleteMessage { at, message, now } => {
                let Some(user) = self.my_user_id else {
                    tracing::warn!("Logic: delete on {channel_id} before the session; dropped");
                    return;
                };
                // The files the message alone carried, read before the
                // delete takes them off it (`ambre-spec-murmure-attachments.md`).
                let files = match at {
                    MessageAt::Segment(segment) => channel.erasable(segment, message, user),
                    MessageAt::Thread(_) => Vec::new(),
                };
                let meta = channel.meta_id().document_id();
                let deleted = channel.delete(at, message, user);
                if deleted.is_ok() {
                    self.erase_files(meta, files);
                }
                let Some(channel) = self.channels.get_mut(&channel_id) else {
                    return;
                };
                // A deleted reply leaves the count of its root (the Murmure
                // threads spec), and its copy goes with it.
                let (head, copy) = match (&deleted, at) {
                    (Ok(_), MessageAt::Thread(root)) => {
                        (channel.root_view(root), channel.copy_of(root, message))
                    }
                    _ => (None, None),
                };
                self.send_change(channel_id, deleted.map(Some));
                if let Some(event) = head {
                    self.emit(UiEvent::Channel { channel_id, event });
                }
                if let Some((segment, copy)) = copy {
                    self.handle_channel_command(
                        channel_id,
                        MurmureCommand::DeleteMessage {
                            at: segment.into(),
                            message: copy,
                            now,
                        },
                    );
                }
            }
            MurmureCommand::Reply {
                root,
                draft,
                also_in_channel,
                now,
                ..
            } => {
                // A reply is screened as a message is: a copy to the channel
                // notifies it as one (the Murmure threads spec).
                let members: Option<Vec<Id<User>>> = self
                    .channel_members
                    .get(&channel_id)
                    .map(|rows| rows.iter().map(|row| row.user_id).collect());
                let draft = members_only(draft, members.as_deref());
                let Some(author) = self.my_user_id else {
                    tracing::warn!("Logic: reply on {channel_id} before the session; dropped");
                    return;
                };
                match channel.reply(root, &draft, author, now, also_in_channel) {
                    Ok(replied) => self.send_reply(channel_id, replied, now),
                    Err(error) => {
                        tracing::warn!("Logic: reply on channel {channel_id} refused: {error}");
                    }
                }
            }
            MurmureCommand::Tick { now } => {
                let meta = channel.meta_id().document_id();
                // A successor that failed to open is fetched again past its
                // delay (the Murmure segments spec).
                if let Some(successor) = channel.successor_to_retry(now) {
                    self.ask(open_in_channel(meta_id, successor.document_id()));
                }
                self.drive_finalization(channel_id, now);
                self.drive_fold(channel_id, now);
                // A close a rotation owes waits for no send (the Murmure
                // channels spec).
                if self
                    .channels
                    .get(&channel_id)
                    .is_some_and(ChannelState::owes_rotation_close)
                {
                    self.close_if_full(channel_id, now);
                }
                let watchers = self.channel_watchers.entry(channel_id).or_default();
                let changed = watchers.tick(now);
                if watchers.owes_watching(now) {
                    self.send_sample(meta, Sample::Watching);
                }
                if changed {
                    self.show_watchers(channel_id);
                }
                self.replay_owed(channel_id, now);
                self.ask_call_rosters(channel_id, now);
            }
            MurmureCommand::Typing { now, thread } => {
                let meta = channel.meta_id().document_id();
                if self
                    .channel_watchers
                    .entry(channel_id)
                    .or_default()
                    .owes_typing(now, thread)
                {
                    self.send_sample(meta, Sample::typing(thread));
                }
            }
        }
    }

    // --- Who watches a channel, and who types in it ---

    /// Send a sample of this user on the `meta` document of a channel, and on
    /// no other. Only its members hold that key (the Murmure typing and
    /// presence spec), and an ephemeral reaches no history.
    fn send_sample(&self, meta: Id<Document>, sample: Sample) {
        self.ask(SyncCommand::SendEphemeral {
            document_id: meta,
            payload: sample.bytes(),
        });
    }

    /// Hand the view who watches a channel and who types in it.
    fn show_watchers(&self, channel_id: Id<Channel>) {
        let Some(watchers) = self.channel_watchers.get(&channel_id) else {
            return;
        };
        let (watching, typing) = watchers.view();
        let in_threads = watchers.in_threads();
        self.emit(UiEvent::Channel {
            channel_id,
            event: MurmureEvent::Watchers {
                watching,
                typing,
                in_threads,
            },
        });
    }

    /// Sync listed the members of a document again. Returns the rows back
    /// when no channel, open or opening, has it as its `meta`. An open
    /// channel hands them to the view, and an opening one at its open.
    pub fn channel_members_changed(
        &mut self,
        document_id: Id<Document>,
        members: Vec<MemberInfo>,
    ) -> Option<Vec<MemberInfo>> {
        let opening = self.opening.iter().find_map(|(&channel_id, opening)| {
            opening
                .as_ref()
                .filter(|o| o.meta_id().document_id() == document_id)
                .map(|_| channel_id)
        });
        let Some(channel_id) = self.channel_of_meta(document_id).or(opening) else {
            return Some(members);
        };
        self.channel_members.insert(channel_id, members);
        if self.channels.contains_key(&channel_id) {
            self.show_channel_members(channel_id);
        }
        None
    }

    /// Hand the view the members of a channel, once Sync listed them.
    fn show_channel_members(&self, channel_id: Id<Channel>) {
        if let Some(members) = self.channel_members.get(&channel_id) {
            self.emit(UiEvent::Channel {
                channel_id,
                event: MurmureEvent::Members {
                    members: members.clone(),
                },
            });
        }
    }

    /// The open channel whose `meta` is `document_id`.
    pub(crate) fn channel_of_meta(&self, document_id: Id<Document>) -> Option<Id<Channel>> {
        self.channels
            .iter()
            .find(|(_, channel)| channel.meta_id().document_id() == document_id)
            .map(|(&channel_id, _)| channel_id)
    }

    /// An ephemeral on a document. Returns `false` when it is no `meta` of an
    /// open channel. A payload that is no sample is dropped without an error
    /// (`PRESENCE-013`), and so is a sample of this user's.
    ///
    /// A member heard for the first time gets this user's watching sample at
    /// once, so a newcomer sees who watches without waiting a refresh period.
    pub fn channel_sample_heard(
        &mut self,
        document_id: Id<Document>,
        from: Id<User>,
        payload: &ambre_core::Payload<ambre_core::EphemeralBytes>,
    ) -> bool {
        let Some(channel_id) = self.channel_of_meta(document_id) else {
            return false;
        };
        let Some(sample) = Sample::read(payload) else {
            return true;
        };
        if Some(from) == self.my_user_id {
            return true;
        }
        let watchers = self.channel_watchers.entry(channel_id).or_default();
        let newcomer = watchers.is_new(from);
        if watchers.heard(from, sample) {
            self.show_watchers(channel_id);
        }
        if newcomer {
            self.send_sample(document_id, Sample::Watching);
        }
        true
    }

    /// The relay says a peer stopped watching a document. Returns `false`
    /// when it is no `meta` of an open channel.
    ///
    /// Another connection of this user leaving turns no one away: this one
    /// answers with its sample, so the other members still see it.
    pub fn channel_peer_left(&mut self, document_id: Id<Document>, user: Id<User>) -> bool {
        let Some(channel_id) = self.channel_of_meta(document_id) else {
            return false;
        };
        if Some(user) == self.my_user_id {
            self.send_sample(document_id, Sample::Watching);
        } else if self
            .channel_watchers
            .get_mut(&channel_id)
            .is_some_and(|watchers| watchers.left(user))
        {
            self.show_watchers(channel_id);
        }
        true
    }

    /// Hand the delta of a reaction, an edit or a delete to the Sync worker
    /// with its summary. The view takes the segment's messages (the Murmure
    /// reactions and messages specs). A change that wrote nothing sends
    /// nothing.
    fn send_change(&self, channel_id: Id<Channel>, reacted: Result<Option<Sent>, MurmureError>) {
        match reacted {
            Ok(Some(sent)) => {
                self.ask(SyncCommand::SendUpdate {
                    document_id: sent.document,
                    payload: sent.delta,
                    summary: Some(sent.summary),
                });
                self.emit(UiEvent::Channel {
                    channel_id,
                    event: sent.event,
                });
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!("Logic: change on channel {channel_id} refused: {error}");
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
    fn mark_unread(&mut self, channel_id: Id<Channel>, segment: Id<Segment>, message: Id<Message>) {
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
                .map(|later| (later, Seq::<Relay>::ZERO)),
        );
        for &(moved, read) in &cursors {
            self.read_cursors.insert(
                moved.document_id(),
                ReadCursor {
                    read,
                    active: ambre_bus::Activity::Active,
                },
            );
        }
        self.ask(SyncCommand::MoveReadCursors { cursors });
        self.emit_count(channel_id);
    }

    /// Hand the view the relay's activity answer, one mark per channel: a
    /// channel is active when any of its documents is (the Murmure
    /// unread spec).
    ///
    /// Opens nothing: the answer is a yes or no per document, with this
    /// user's own cursor on it. An open channel counts its unread messages
    /// from those cursors.
    pub fn channel_activity(&mut self, documents: &[DocumentActivity]) {
        let mut channels: Vec<ChannelActivity> = Vec::new();
        let mut index: HashMap<Id<Channel>, usize> = HashMap::new();
        for document in documents {
            self.document_channels
                .insert(document.document_id, document.channel_id);
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
                        channel.active = channel.active.max(document.active);
                    }
                }
                None => {
                    index.insert(document.channel_id, channels.len());
                    channels.push(ChannelActivity {
                        channel_id: document.channel_id,
                        active: document.active,
                        unread: None,
                        mentions: None,
                    });
                }
            }
        }
        let mut counted_now = Vec::new();
        for mark in &mut channels {
            if let Some(counted) = self.counted(mark.channel_id) {
                *mark = counted;
                counted_now.push((mark.channel_id, mark.unread));
            }
        }
        self.emit(UiEvent::ChannelActivity { channels });
        for (channel_id, unread) in counted_now {
            self.emit_counted_first_unread(channel_id, unread);
        }
        self.segments_listed(documents);
        self.place_threads();
    }

    /// Apply remote payloads to the channel that holds `document_id`
    /// (the Murmure engine spec), each with where the relay placed it.
    /// Returns `false` when no open channel holds it, so the caller reports
    /// the drop as it does for a document.
    pub fn apply_channel_remote(
        &mut self,
        document_id: Id<Document>,
        payloads: &[(Option<Order>, ambre_core::LoroPayloadRef<'_>)],
    ) -> bool {
        // A channel still opening keeps the bytes until the document is ready.
        if let Some(opening) = self
            .opening
            .values_mut()
            .flatten()
            .find(|o| o.expects(document_id))
        {
            for &(order, payload) in payloads {
                trace_receipt(document_id, opening.receive_at(document_id, order, payload));
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
        if !events.is_empty() {
            for event in threads::latest_answers(events) {
                self.emit(UiEvent::Channel { channel_id, event });
            }
            self.emit_count(channel_id);
        }
        self.follow_close(channel_id);
        true
    }

    /// Apply signed deltas to the channel that holds `document_id`, each
    /// through the authorship check of the engine (the Murmure engine spec).
    /// A refused delta changes nothing, and still counts as delivered. One
    /// from a newer build asks the user to update. Returns `false` when no
    /// open or opening channel holds the document.
    pub fn apply_channel_signed(
        &mut self,
        document_id: Id<Document>,
        deltas: &[SignedDelta],
    ) -> bool {
        let signer = |delta: &SignedDelta| Signer {
            nonce: delta.nonce,
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
                let kept =
                    opening.receive_signed(document_id, delta.seq, &delta.payload, signer(delta));
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
                    delta.seq,
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
        let mut fresh = None;
        let mut cues = Vec::new();
        for (delta, verdict) in deltas.iter().zip(verdicts) {
            let accepted = verdict.is_ok();
            if accepted && open.is_some() {
                fresh = self.news_of(delta).map_or(fresh, |mention| {
                    Some(mention.max(fresh.unwrap_or_default()))
                });
                cues.extend(self.cue_of(delta));
            }
            self.record_channel_seq(document_id, delta.seq, accepted.then_some(&delta.payload));
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
            self.emit(UiEvent::UpdateRequired);
        }
        // One segment answers with its whole list, so only the last answer of
        // a batch is news to the view.
        if let Some(channel_id) = open.filter(|_| !events.is_empty()) {
            for event in threads::latest_answers(events) {
                self.emit(UiEvent::Channel { channel_id, event });
            }
            self.emit_count(channel_id);
        }
        if let Some(channel_id) = open {
            // Taken on every batch, so a message never outlives its own.
            let message = self
                .channels
                .get_mut(&channel_id)
                .and_then(ChannelState::take_arrived);
            if let Some(mention) = fresh {
                self.emit(UiEvent::NewMessage {
                    channel_id,
                    mention,
                    message,
                });
            }
            for cue in cues {
                self.emit(UiEvent::CallCue { channel_id, cue });
            }
            self.follow_close(channel_id);
        }
        true
    }

    /// The cue of a call a delta of another member carries in its sealed summary, if any
    /// (the Murmure calls spec, the ring). A cue of this user rings none of its own pages.
    pub fn cue_of(&self, delta: &SignedDelta) -> Option<ambre_bus::CallCue> {
        let cue = delta.summary.as_ref()?.call?;
        (delta.user.is_none() || delta.user != self.my_user_id).then_some(cue)
    }

    /// The cue of a call a closed channel's document carried, named to the view under its
    /// channel (the Murmure calls spec, the ring). A document of no closed channel cues
    /// nothing.
    pub fn cue_closed(&mut self, document_id: Id<Document>, cue: ambre_bus::CallCue) {
        let channel_id = self
            .closed_channels
            .iter()
            .find_map(|(channel_id, closed)| closed.holds(document_id).then_some(*channel_id));
        if let Some(channel_id) = channel_id {
            self.emit(UiEvent::CallCue { channel_id, cue });
        }
    }

    /// Whether a delta is a message another member wrote, from its sealed
    /// summary alone (the Murmure notifications spec). `Some(Mentioned)` when
    /// it names this user or the channel. A delta with no summary is none.
    pub fn news_of(&self, delta: &SignedDelta) -> Option<ambre_bus::MentionState> {
        let summary = delta.summary.as_ref()?;
        if summary.kind != ambre_bus::ActivityKind::Content
            || (delta.user.is_some() && delta.user == self.my_user_id)
        {
            return None;
        }
        Some(ambre_bus::MentionState::from_bool(
            summary.mentions_all == ambre_bus::MentionScope::All
                || self
                    .my_user_id
                    .is_some_and(|me| summary.mentions.contains(&me)),
        ))
    }

    /// Search every open channel for `query` (the Murmure search spec). Each
    /// engine reads the segments it holds, and nothing reaches the Sync
    /// worker: the query never leaves the client, and no segment loads.
    pub fn search_messages(&self, query: ambre_core::Validated<ambre_core::SearchQuery>) {
        let mut channels: Vec<ChannelMatches> = match Query::parse(&query) {
            Some(parsed) => self
                .channels
                .values()
                .map(|channel| channel.search(&parsed))
                .collect(),
            None => Vec::new(),
        };
        channels.sort_by_key(|matches| matches.channel_id.to_string());
        self.emit(UiEvent::MessageSearch { query, channels });
    }
}
